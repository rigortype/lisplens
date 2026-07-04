# Feedback: comment trivia for inline-comment alignment — NO ACTION NEEDED (already exposed by `lex()`)

Investigated while assessing whether lisplens needs anything more from lispexp after
the Phel 0.7.0 consumption (feedback 0003–0006). This note records a *candidate*
upstream ask and its disposition: **it is not needed** — lispexp already provides it.

## The candidate ask

lisplens's formatter has one remaining cross-cutting limitation: it does not align a
single **inline** (trailing) `;` comment, and comment-**only** line indentation is the
shared driver's guess (it can differ from cljfmt / `phel format`). The natural
assumption is that this needs an upstream change — that the reader drops comments, so
a round-trip formatter can't see them.

## Why no change is needed

That assumption is wrong. lispexp already exposes comments as **lexer trivia**:

- `lispexp::lex(source, &options) -> Lexer` is a public token iterator (the `Lexer`
  struct), yielding `TokenKind::LineComment` and `TokenKind::BlockComment` tokens
  **with byte spans** (see the module doctest in lispexp's `lexer.rs`, which filters
  the stream on `TokenKind::LineComment`).
- From those spans plus a `LineIndex`, a consumer can classify each comment as
  own-line vs. trailing (is there code before it on the same line?) and align it —
  no new reader/parser API required.

The `parse()` → `Datum` tree deliberately omits comments (they are not data — the
right default for evaluators and structural queries). A formatter that wants comment
fidelity should consume the **lexer** alongside the tree, not ask the tree to carry
comments.

## Disposition

- **On lispexp:** nothing to do. The trivia is already there and sufficient. (A
  *convenience* — comment tokens surfaced on the tree, or a ready-made "comments with
  attachment" view — would be ergonomic sugar, not a capability gap; not requested.)
- **On lisplens:** the inline-comment-alignment / comment-line-indentation work is a
  lisplens-side TODO (it would consume `lex()`), tracked in the formatter's Deferred
  list (`docs/dev/formatter.md`, `docs/CURRENT_WORKS.md`). It is **not** blocked on
  lispexp.

Recorded so a future contributor does not file this as an upstream need — the answer
is already "use `lex()`."
