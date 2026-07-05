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
- **On lisplens:** **done** — the formatter now consumes `lex()` for comment-line
  handling (`comment_only_lines` in `src/format/mod.rs`). The verified fidelity gap
  turned out to be *over*-reindenting: the Clojure engine (Clojure/Phel/…) now leaves
  comment-only lines where written, matching `cljfmt`/`phel format` byte-exact, using
  the lexer to classify each dialect's comment char (`;`, Janet's `#`). Trailing-comment
  column *alignment* was declined — Emacs `indent-region`, `cljfmt`, and `phel format`
  all leave trailing comments untouched, so the prior byte-preserving behaviour was
  already faithful.

Recorded so a future contributor does not file this as an upstream need — the answer
was "use `lex()`", now consumed.
