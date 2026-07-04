# lispexp feedback

Upstream reader/lexer/API needs that lisplens has found while building on
[lispexp](https://github.com/rigortype/lispexp). Per the AGENTS ground rule, lisplens
**records** these here for the lispexp maintainer to implement upstream — it never
edits, commits to, or opens a PR against the lispexp repo. Each note states the
phenomenon, the confirmed diagnosis, and a concrete proposed fix; lisplens consumes
a published version afterward and marks the note resolved.

| # | Topic | Severity | Status |
| --- | --- | --- | --- |
| [0001](0001-lineindex-byte-api-vs-line-normalization.md) | `LineIndex` byte API vs. normalized `line_range` | low | open (workaroundable; API/doc note) |
| [0002](0002-improper-list-dot-span.md) | expose the improper-list `.` separator span | — | **resolved** (lispexp 0.5.0) |
| [0003](0003-discarded-forms-dropped.md) | `#_` / `#;` discarded forms are dropped from the tree | low | **resolved** (lispexp 0.7.0 — `Options.keep_discarded`) |
| [0004](0004-phel-semicolon-in-symbol.md) | Phel reader splits a symbol at an interior `;` | low | **resolved** (lispexp 0.7.0 — `Options.line_comment_in_atom`, Phel) |
| [0005](0005-phel-pipe-anonymous-function.md) | Phel `\|(…)` anonymous-function reader macro unrecognized | medium | **resolved** (lispexp 0.7.0 — `Options.pipe_anon_fn`, Phel) |
| [0006](0006-phel-fqn-vs-char-literal.md) | Phel PHP FQN `\Foo\Bar` mis-read as `\`-char literals | medium | **resolved** (lispexp 0.7.0 — `CharSyntax::BackslashFqn`, Phel) |
| [0007](0007-comment-trivia-already-in-lexer.md) | comment trivia for inline-comment alignment | — | **no action needed** (already exposed by `lex()`) |

When a note is resolved upstream, update its status here and mark the note's title
(see 0002 for the pattern), then drop the matching lisplens-side workaround.

## Consumption assessment — lispexp 0.7.0 (2026-07-04)

After consuming lispexp 0.7.0 + lispexp-emacs 0.2.0, lisplens re-checked whether the
reader is sufficient for everything it does. **Verdict: sufficient.** No new blocking
upstream need; the only note still open is 0001 (low, workaroundable — see its
Disposition section).

Evidence:

- **Structural parse:** 0 parse errors across all 260 phel-lang `.phel` files and 373
  Clojure `.clj/.cljs/.cljc` files (`lisplens check`).
- **Formatting:** byte-exact code-line indentation vs `cljfmt fix` on 373 Clojure files
  (semantic style, zero divergences — the former `#_`-discard residuals resolved by
  0003) and vs `phel format` on the phel-lang corpus.
- **Symbol accuracy:** `refs` / `rename` are correct around Phel PHP FQNs and sibling
  symbols — e.g. renaming `foo` leaves `foobar`, `\RuntimeException`, and
  `\Phel\Lang\Symbol/create` untouched — confirming 0005/0006 give a
  structurally-correct tree, not just correct indentation.

Two residuals surfaced during the Phel validation; **neither is a lispexp gap**, so
neither becomes a note:

1. **`phel format` quirk — closing `])` after a trailing comment.** In a `defstruct`
   field vector where the last field carries a trailing `;` comment, `phel format`
   places the closing `])` at column 0; lisplens aligns it under the fields. With the
   comments removed the two agree exactly, so this is an oracle (`phel format`) edge
   case — lisplens is the more consistent one. Not a reader issue.
2. **`phel format` semantic failure — `::alias/kw` without the alias.** `phel format`
   refuses `tests/phel/keyword-alias-resolution.phel` with "Can not resolve alias …"
   (it does namespace-alias resolution); lisplens formats it fine. A `phel format`
   limitation, not a lispexp one.

The one remaining lisplens-side formatter limitation (inline-comment alignment) needs
nothing from lispexp — see 0007.
