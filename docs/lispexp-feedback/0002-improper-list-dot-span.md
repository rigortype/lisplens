# Feedback: expose the dot separator span of an improper list — RESOLVED (lispexp 0.5.0)

Upstream request from lisplens (a **verbatim / round-trip** consumer). **Shipped
in lispexp 0.5.0** (2026-07-03) and consumed by the formatter the same day.
Origin: [../notes/20260703-lispexp-feedback.md](../notes/20260703-lispexp-feedback.md).

## The gap (lispexp ≤ 0.4)

`DatumKind::List { delim, items, tail: Option<Box<Datum>> }` gave the tail
*datum* of an improper list but no span for the `.` separator. Emacs's indenter
is text-based and treats a lone `.` as an alignable token: in the
font-lock-keywords idiom `'(eval . FORM)`, a continuation line of the tail aligns
*under the `.`*, not under the car. Example, `dired.el`:

```elisp
'(eval .
       ;; It is quicker to first find just an extension ...
       (list (concat ...)))     ; Emacs aligns these under the `.` (col 10)
```

The formatter had `items = [eval]` and a `tail` but no `.` position, so it
aligned the continuation under `eval` (col 5). The only route to the dot column
was to re-scan the source between `items.last().span.end` and `tail.span.start` —
re-doing work the reader had already done.

## The fix (lispexp 0.5.0)

A fourth field on the list kind plus a reader:

```rust
DatumKind::List { delim, items, tail, dot: Option<Span> }
impl Datum<'_> { pub fn dot_span(&self) -> Option<Span>; }
```

- `dot` is `Some` iff `tail` is `Some`; it is the byte span of the consumed `.`
  token (no rescan cost).

**Breaking**, not additive as originally hoped: `DatumKind` is deliberately not
`#[non_exhaustive]` (consumers *want* the exhaustive-match guarantee), so a new
field breaks any full-field `List { delim, items, tail }` pattern. In lisplens
this cost nothing — every `List { … }` match already used `..` — so the only
change was `Cargo.toml` (`lispexp = "0.5"`).

## What lisplens does with it

`format::normal_indent` (see [../dev/formatter.md](../dev/formatter.md)): for a
lone-car dotted pair whose `.` sits on the open-paren line, align the tail's
continuation under `dot_span().start` instead of under the car. Verified
byte-exact against Emacs `indent-region`; `dired.el` dropped 53→35 harness diffs
with 0 regressions across the emacs `lisp/` and magit/lem corpora.

## Companion request (comment spans) — no upstream change needed

The notes' lower-priority ask (comment spans, for `indent-for-comment` on a lone
`;` line) is already served by the **`lex` token layer**, which emits
`LineComment` / `BlockComment` with byte spans — the tree stays trivia-free by
design and a consumer correlates via span. lisplens has not needed it yet
(`;;;`-preserve and `whitespace-after-open-paren` are handled from the datum
tree); if/when a lone-`;` `comment-column` pass is wanted, filter `lex(src, opts)`
rather than adding a `Parsed.comments` side channel. Left open, low priority.
