# Handoff: lispexp feedback from the Emacs Lisp formatter work (2026-07-03)

Handoff note for whoever picks up the upstream conversation. Captures what the
formatter fidelity work surfaced about the [lispexp](https://crates.io/crates/lispexp)
backend: what worked, the one concrete request, and a lower-priority nice-to-have.
Durable formatter knowledge lives in [../dev/formatter.md](../dev/formatter.md);
the established upstream channel is [../lispexp-feedback/](../lispexp-feedback/).

## Context

While closing the formatter's indentation long tail (data lists vs calls,
specform bodies, dotted tails, `;;;` comments, `whitespace-after-open-paren`) and
adding Nameless-aware indentation (ADR-0030), the formatter leaned hard on
lispexp. `php-mode/lisp` is now effectively 100% faithful; most *remaining*
harness diffs are the batch-Emacs reference lacking a file's own indent specs,
not lisplens errors (see [[formatter-harness-declare-caveat]] and CURRENT_WORKS).

## What worked well (no change wanted — affirmations)

- **`indent::harvest_indent_specs`** is the backbone of the fidelity story. It
  covers `(declare (indent …))`, `(put 'sym 'lisp-indent-function …)`,
  `function-put`, and `lisp-indent-hook` (confirmed at `lispexp/src/indent.rs`
  around `harvest_put` / `harvest_declare`). It correctly picked up file-local
  macros like `mpc-select-save` (`declare (indent 0)`) and `jsonrpc-lambda`, which
  is exactly why lisplens matches the checked-in file where batch Emacs (which
  never evaluates the file) does not. `IndentSpec::{Number,Defun,Function,Raw}`
  is a good, faithful shape.
- **Char-literal lexing is robust.** `? ` (the space char in `rx`'s `(? …)`),
  `?x`, and tricky forms like `?\(` all classify as `DatumKind::Char` without
  desyncing the reader. This let `head_is_symbol_like` reproduce Emacs's
  `\sw\|\s_` first-char test purely by `DatumKind`, no source peeking.
- **Improper lists stay surface-faithful** — `(a . (b c))` is kept as
  `List { items:[a], tail: Some((b c)) }`, not normalized to `(a b c)`. Correct
  for a surface-preserving formatter (it must reindent the dotted form as written).

## Request (one, low severity) — expose the dot token's span

**Confirmed fact (lispexp src/datum.rs):** an improper list is
`DatumKind::List { delim, items, tail: Option<Box<Datum>> }`. `tail` gives the
tail *datum*, but there is **no span for the `.` separator** itself.

**Why it matters:** Emacs's indenter is text-based and treats a lone `.` as an
alignable token. For `'(eval . FORM)` — the common font-lock-keywords idiom — a
continuation line of the tail aligns *under the `.`*, not under the car. Real
example, `~/local/src/emacs/lisp/dired.el`:

```elisp
'(eval .
       ;; It is quicker to first find just an extension ...
       (list (concat ...)))     ; Emacs aligns these under the `.` (col 10)
```

lisplens has `items = [eval]` and a `tail`, but no `.` position, so it aligns the
tail continuation under `eval` (col 5) instead. To match Emacs byte-for-byte the
formatter needs the `.`'s column; today the only route is to re-scan the source
between `items.last().span.end` and `tail.span.start` for the `.`, re-doing work
the reader already did.

**Ask (additive, non-breaking):** expose the separator span, e.g.
`List { …, dot: Option<Span> }`, or a helper `Datum::dot_span() -> Option<Span>`.

**Severity: low.** The idiom is niche, and note that even real code doesn't
follow the quirk consistently (dired.el's own comment sits at col 5 while Emacs
would reindent it to col 10). lisplens currently just leaves this one case
slightly off rather than shipping a source-rescan hack. This is the *only* gap
where lisplens can't reach pure text-based Emacs behavior.

## Nice-to-have (lower priority, already worked around) — comment spans as trivia

**Confirmed fact:** `Parsed { lang_line, data, errors }` drops comments and
whitespace (`lispexp/src/lib.rs`: "The tree drops comments and whitespace").
`Options` knows `line_comment`/`block_comment`/`datum_comment`, but comment
*ranges* are not surfaced.

**Where the formatter felt it:**
- `;;;` (3+ semicolon) lines are "left in place" by Emacs — detected textually
  (`trimmed.starts_with(";;;")`), fine.
- `whitespace-after-open-paren` is detected via the span gap
  (`first.span.start > open.span.start + 1`), fine.
- **Not implemented:** a lone `;` comment on its own line should go to
  `comment-column` (`indent-for-comment`). Doing this principled-ly needs to know
  a comment is there and its column — which the tree doesn't provide.

**Ask (optional):** a side channel on `Parsed`, e.g. `comments: Vec<Span>` (or a
trivia iterator), so a formatter can indent comments faithfully without
re-lexing. Not a blocker — the two common cases are already handled textually.

## Suggested next action

Write the dot-span request up as `docs/lispexp-feedback/0002-improper-list-dot-span.md`
in the same shape as `0001` (Confirmed facts → the sharp edge → lisplens's
decision → additive proposal), with the comment-spans item as a short low-priority
appendix. This handoff has the grounded facts and file references to lift from.
