# Feedback: Phel PHP fully-qualified names (`\Foo\Bar`) are mis-read as `\`-char literals — RESOLVED (lispexp 0.7.0)

**Shipped in lispexp 0.7.0** as `CharSyntax::BackslashFqn`, set for the Phel preset. `\RuntimeException`, `\Phel\Lang\Symbol`, `(php/new \RuntimeException)` now read as single symbols (correct datum kind and child count for multi-segment names); genuine char literals (`\newline`, `\a`) still read. Validated upstream against phel-lang with 648 FQNs correctly structured; lisplens consumes it via `Options::for_dialect(Dialect::Phel)` and formats `(\Phel\Lang\Symbol/create "s")` byte-exact vs `phel format`. Regression golden in `src/format/clojure.rs`.

Upstream feedback to [lispexp](https://crates.io/crates/lispexp) from lisplens. Severity: **medium** — like the `|(…)` gap (0005) it does not (much) break the formatter, but it mis-structures a **very common** Phel form (PHP interop uses `\Foo\Bar` class names everywhere), which is wrong for any symbol-accurate operation. Found while validating lisplens' Phel indenter against `phel format`.

## Confirmed facts (lispexp 0.6, `Dialect::Phel`)

Phel writes PHP fully-qualified names with a leading/inner backslash: `\RuntimeException`, `\Phel\Lang\Symbol`, `(php/new \RuntimeException …)`. Phel reads each as **one symbol** (the FQN), *not* as a character literal.

lispexp's Phel reader (which inherits Clojure's `\c` character syntax) instead lexes them as `\`-char literals. Parse dumps:

```
(foo \RuntimeException)     → [ Symbol("foo"),  Char("\RuntimeException") ]
(foo \Phel\Lang\Symbol)    → [ Symbol("foo"),  Char, Char, Char ]      ← one FQN split into three chars
```

Phel yields `[ Symbol("foo"), Symbol("\Phel\Lang\Symbol") ]` for the second — a single symbol.

The single-segment case (`\RuntimeException`) still occupies exactly one slot, so *argument alignment* is unaffected (which is why the formatter corpus barely notices — 307/310 byte-exact); but the datum is the **wrong kind** (`Char`, not `Symbol`), and a **multi-segment** FQN also changes the child count. Both break refs/rename/extract, which act on the tree.

## Cause and what would resolve it

Phel's own lexer already disambiguates this — its character-literal rule (`Compiler/Application/Lexer.php`) is

```
\(?:space|newline|tab|…|u[0-9a-fA-F]{4}|o[0-7]{1,3}|[^\s])(?![A-Za-z0-9_\-\\])
```

The trailing negative lookahead **`(?![A-Za-z0-9_\-\\])`** is the key: a `\c` is a character only when *not* followed by an identifier character or another backslash — otherwise it "falls through to atom … preserving FQN parsing for `\Phel\Lang\Symbol`" (Phel's own comment).

lispexp's Phel char syntax needs the same boundary guard: read `\c` as a char literal only when the char is a known named char (`\space`, `\newline`, `\uNNNN`, …) or a single char **not** followed by `[A-Za-z0-9_\-\\]`; otherwise the whole `\Foo\Bar` run is a symbol. This is Phel-specific — Clojure's `\c` doesn't need it — so it wants a dedicated Phel char-syntax mode (or a flag on the existing `CharSyntax::Backslash`), not a change to Clojure.

## lisplens's stance

Not blocking the formatter today, but PHP-interop FQNs are pervasive in real Phel, so this (with 0005, `|(…)`) is the higher-value of the Phel reader gaps — worth fixing before lisplens exposes symbol-accurate edits on `.phel`. Recorded here per the AGENTS ground rule (lisplens records upstream needs; it does not PR lispexp).
