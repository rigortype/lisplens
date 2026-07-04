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

When a note is resolved upstream, update its status here and mark the note's title
(see 0002 for the pattern), then drop the matching lisplens-side workaround.
