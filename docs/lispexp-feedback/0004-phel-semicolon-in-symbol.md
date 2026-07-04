# Feedback: Phel reader splits a symbol at an interior `;` (treats it as a comment) — RESOLVED (lispexp 0.7.0)

**Shipped in lispexp 0.7.0** as `Options.line_comment_in_atom`, set for the Phel preset (`Options::phel()`). A `;` inside a token is now a symbol constituent (`foo;bar` is one symbol, `'*_.%;!:+-?` reads whole) while a boundary `;` stays a comment. lisplens consumes it automatically via `Options::for_dialect(Dialect::Phel)`. Confirmed: all 260 phel-lang `.phel` files parse clean (0 errors), and a symbol-with-interior-`;` no longer truncates the following forms. Regression golden in `src/format/clojure.rs`.

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

This is what Phel's own lexer does. Its atom pattern (`Compiler/Application/Lexer.php`) is `([^\(\)\[\]\{\},`@ \n\r\t\#]+\#?)` — the exclusion set does **not** contain `;`, so `;` is an atom constituent; a separate, earlier-ordered comment rule (`;[^\n]*`) only wins when a token *starts* with `;`.

## What would resolve it (proposed)

Add a dialect option — `Options.line_comment_in_atom: bool`, default `false` — meaning the `line_comment` character is an ordinary atom constituent, so it terminates a symbol only at a token boundary. Set it in `Options::phel()` (`Options { line_comment_in_atom: true, ..Options::clojure() }`); every other dialect keeps `;` as a terminator.

The lexer change is one clause in the atom-terminator test: `|| (c == self.opts.line_comment && !self.opts.line_comment_in_atom)`. The comment-at-token-start path is untouched — `next_token` already lexes a leading `;` as a comment before any atom is started — so a `;` only comments when it *begins* a token (`foo ;bar`), while `foo;bar` and `'*_.%;!:+-?` read whole. (`Options` is `#[non_exhaustive]`, so the field is additive/non-breaking.)

## lisplens's stance

Not blocking — lisplens documents it as a known Phel formatter limitation (`docs/dev/formatter.md`), alongside the `#_`-discard and comment-only-line notes. It affects only symbols containing `;` (essentially symbol-edge-case tests); ordinary Phel code is unaffected, and lisplens is otherwise byte-exact with `phel format` on 307/310 of phel-lang's own files.
