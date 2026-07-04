# Feedback: Phel reader splits a symbol at an interior `;` (treats it as a comment)

Upstream feedback to [lispexp](https://crates.io/crates/lispexp) from lisplens. Severity: **low** (rare — only deliberate symbol edge-case tests — but it cascades a whole form's indentation once it mis-parses). Found while validating lisplens' Phel indenter against `phel format`.

## Confirmed facts (lispexp 0.6, `Dialect::Phel`)

Phel treats `;` as a line comment **only at a token boundary**. Once a symbol token has started, `;` is an ordinary symbol constituent — Phel's own tests rely on this:

```phel
(is (= '*_.%;!:+-? (symbol "*_.%;!:+-?")) "symbol on complex string")
```

Here `'*_.%;!:+-?` is a single quoted symbol; `phel format` (and the Phel reader) parse the line as balanced.

lispexp's Phel reader instead breaks the symbol at `;`: for `'*x;y` it reads `'*x` and then treats `;y` as a comment to end of line. Reduced:

```
input:   (is (= '*x;y z))
                (is (= 1 1))
```

lispexp reads the first `(is …` as **unterminated** (the `;y z))` is eaten as a comment), so every following top-level form is mis-nested — lisplens then indents the next `(is …)` as if still inside the first, a full cascade.

`;`-as-comment does apply at a boundary in Phel (e.g. `headers ; Map with all headers` in `http.phel`), so the rule is specifically: **`;` does not terminate an in-progress symbol token**.

## What would resolve it

In the Phel dialect's tokeniser, let `;` be a symbol constituent when it appears mid-token (no preceding whitespace / delimiter), and only start a comment at a token boundary — matching Phel's own lexer.

## lisplens's stance

Not blocking — lisplens documents it as a known Phel formatter limitation (`docs/dev/formatter.md`), alongside the `#_`-discard and comment-only-line notes. It affects only symbols containing `;` (essentially symbol-edge-case tests); ordinary Phel code is unaffected, and lisplens is otherwise byte-exact with `phel format` on 307/310 of phel-lang's own files.
